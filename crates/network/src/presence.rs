use std::time::Duration as StdDuration;

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loontail_core::auth::AuthUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::models::{PresenceStatus, UserDto};
use loontail_core::AppState;
use loontail_core::Metrics;
use loontail_core::ServerEvent;

/// Collapse a stored status to `offline` once the last heartbeat exceeds the timeout.
pub fn effective_status(
    stored: PresenceStatus,
    last_heartbeat: DateTime<Utc>,
    timeout: StdDuration,
) -> PresenceStatus {
    let timeout = Duration::from_std(timeout).unwrap_or_else(|_| Duration::seconds(60));
    if Utc::now().signed_duration_since(last_heartbeat) > timeout {
        PresenceStatus::Offline
    } else {
        stored
    }
}

/// A friend who is an active guest in a friends-of-friends world surfaces as
/// in-world pointing at that world, so the viewer can ask to join. Host-only
/// worlds never pass a `guest_world_id` here, so the guest stays plain online.
fn derive_friend_status(
    base: PresenceStatus,
    guest_world_id: Option<Uuid>,
    current_world_session_id: Option<Uuid>,
) -> (PresenceStatus, Option<Uuid>) {
    match guest_world_id {
        Some(world) if base != PresenceStatus::Offline => (PresenceStatus::InWorld, Some(world)),
        _ if base.is_in_world() => (base, current_world_session_id),
        _ => (base, None),
    }
}

#[derive(sqlx::FromRow)]
struct PresenceRow {
    status: String,
    last_heartbeat_at: DateTime<Utc>,
}

pub async fn effective_status_for(state: &AppState, user_id: Uuid) -> AppResult<PresenceStatus> {
    let row = sqlx::query_as::<_, PresenceRow>(
        "SELECT status, last_heartbeat_at FROM presence WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(match row {
        Some(row) => effective_status(
            PresenceStatus::from_db(&row.status),
            row.last_heartbeat_at,
            state.config.heartbeat_timeout,
        ),
        None => PresenceStatus::Offline,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub status: PresenceStatus,
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

/// Mark a user online: only flips an offline row to online, preserving in-world.
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

pub async fn mark_offline(state: &AppState, user_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "UPDATE presence SET status = 'offline', current_world_session_id = NULL, updated_at = now() WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

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

/// Tell a user's friends to reload their list after this user's presence changed.
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
    let status = PresenceStatus::from_client(&body.status)
        .ok_or_else(|| AppError::BadRequest("status must be online, inWorld or joinable".into()))?;

    // Entering a world requires it to be an open world owned by this user.
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

    if let Err(e) = broadcast_presence(&state, auth.id()).await {
        tracing::warn!(error = %e, user_id = %auth.id(), "failed to broadcast presence after status change");
    }

    Ok(Json(StatusResponse { status }))
}

#[derive(sqlx::FromRow)]
struct FriendPresenceRow {
    id: Uuid,
    minecraft_uuid: Option<String>,
    username: String,
    status: Option<String>,
    last_heartbeat_at: Option<DateTime<Utc>>,
    current_world_session_id: Option<Uuid>,
    /// The friend's reported Minecraft version + loader; both must match for the
    /// client to allow joins/invites.
    minecraft_version: Option<String>,
    loader: Option<String>,
    /// The open friends-of-friends world this friend is an active guest in — the
    /// world the viewer may ask to join.
    guest_world_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendPresence {
    #[serde(flatten)]
    pub user: UserDto,
    pub status: PresenceStatus,
    pub current_world_session_id: Option<Uuid>,
    /// The friend's reported Minecraft version + loader; both must match the
    /// viewer's own to allow joins/invites. Null until the friend bootstraps.
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
}

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
                    effective_status(PresenceStatus::from_db(&status), heartbeat, timeout)
                }
                _ => PresenceStatus::Offline,
            };
            let (status, current_world_session_id) =
                derive_friend_status(base, row.guest_world_id, row.current_world_session_id);
            FriendPresence {
                user: UserDto {
                    id: row.id,
                    minecraft_uuid: row.minecraft_uuid,
                    username: row.username,
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

#[cfg(test)]
mod tests {
    use super::{derive_friend_status, effective_status};
    use chrono::{Duration, Utc};
    use loontail_core::models::PresenceStatus;
    use std::time::Duration as StdDuration;
    use uuid::Uuid;

    #[test]
    fn effective_status_keeps_stored_when_within_timeout() {
        let timeout = StdDuration::from_secs(60);
        let fresh = Utc::now() - Duration::seconds(30);
        assert_eq!(
            effective_status(PresenceStatus::Joinable, fresh, timeout),
            PresenceStatus::Joinable
        );
    }

    #[test]
    fn effective_status_collapses_to_offline_past_timeout() {
        let timeout = StdDuration::from_secs(60);
        let stale = Utc::now() - Duration::seconds(120);
        assert_eq!(
            effective_status(PresenceStatus::InWorld, stale, timeout),
            PresenceStatus::Offline
        );
    }

    #[test]
    fn effective_status_boundary_just_inside_timeout_keeps_stored() {
        // A heartbeat just inside the window (timeout - 1s) must NOT collapse.
        let timeout = StdDuration::from_secs(60);
        let boundary = Utc::now() - Duration::seconds(59);
        assert_eq!(
            effective_status(PresenceStatus::Online, boundary, timeout),
            PresenceStatus::Online
        );
    }

    #[test]
    fn derive_friend_status_promotes_live_guest_to_in_world() {
        let world = Uuid::new_v4();
        // An online friend who is an active guest in a FoF world surfaces as in-world,
        // pointing at the guest world (not their own current_world_session_id).
        let other = Uuid::new_v4();
        assert_eq!(
            derive_friend_status(PresenceStatus::Online, Some(world), Some(other)),
            (PresenceStatus::InWorld, Some(world))
        );
    }

    #[test]
    fn derive_friend_status_offline_guest_is_not_promoted() {
        let world = Uuid::new_v4();
        // An offline friend never surfaces a joinable world even if a stale guest row exists.
        assert_eq!(
            derive_friend_status(PresenceStatus::Offline, Some(world), None),
            (PresenceStatus::Offline, None)
        );
    }

    #[test]
    fn derive_friend_status_in_world_without_guest_keeps_own_world() {
        let own = Uuid::new_v4();
        assert_eq!(
            derive_friend_status(PresenceStatus::InWorld, None, Some(own)),
            (PresenceStatus::InWorld, Some(own))
        );
    }

    #[test]
    fn derive_friend_status_online_without_guest_has_no_world() {
        assert_eq!(
            derive_friend_status(PresenceStatus::Online, None, None),
            (PresenceStatus::Online, None)
        );
    }
}
