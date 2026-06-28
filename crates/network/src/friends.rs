use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loontail_core::auth::AuthUser;
use loontail_core::error::{is_unique_violation, AppError, AppResult};
use loontail_core::models::UserDto;
use loontail_core::AppState;
use loontail_core::Metrics;
use loontail_core::ServerEvent;

use crate::presence::{self, FriendPresence};

/// `GET /friends` — the user's friends with their effective presence.
pub async fn list_friends(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<FriendPresence>>> {
    let friends = presence::friends_with_presence(&state, auth.id()).await?;
    Ok(Json(friends))
}

// --- Friend requests -------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFriendRequest {
    pub to_user_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendRequestDto {
    pub id: Uuid,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub from_user: UserDto,
    pub to_user: UserDto,
}

#[derive(sqlx::FromRow)]
struct FriendRequestRow {
    id: Uuid,
    status: String,
    created_at: DateTime<Utc>,
    from_id: Uuid,
    from_minecraft_uuid: Option<String>,
    from_username: String,
    from_avatar_url: Option<String>,
    from_skin_hash: Option<String>,
    to_id: Uuid,
    to_minecraft_uuid: Option<String>,
    to_username: String,
    to_avatar_url: Option<String>,
    to_skin_hash: Option<String>,
}

impl From<FriendRequestRow> for FriendRequestDto {
    fn from(row: FriendRequestRow) -> Self {
        FriendRequestDto {
            id: row.id,
            status: row.status,
            created_at: row.created_at,
            from_user: UserDto {
                id: row.from_id,
                minecraft_uuid: row.from_minecraft_uuid,
                username: row.from_username,
                avatar_url: row.from_avatar_url,
                skin_hash: row.from_skin_hash,
            },
            to_user: UserDto {
                id: row.to_id,
                minecraft_uuid: row.to_minecraft_uuid,
                username: row.to_username,
                avatar_url: row.to_avatar_url,
                skin_hash: row.to_skin_hash,
            },
        }
    }
}

const FRIEND_REQUEST_SELECT: &str = r#"
    SELECT
        r.id, r.status, r.created_at,
        fu.id AS from_id, fu.minecraft_uuid AS from_minecraft_uuid,
        fu.username AS from_username, fu.avatar_url AS from_avatar_url,
        fu.skin_hash AS from_skin_hash,
        tu.id AS to_id, tu.minecraft_uuid AS to_minecraft_uuid,
        tu.username AS to_username, tu.avatar_url AS to_avatar_url,
        tu.skin_hash AS to_skin_hash
    FROM friend_requests r
    JOIN users fu ON fu.id = r.from_user_id
    JOIN users tu ON tu.id = r.to_user_id
"#;

async fn load_request(state: &AppState, id: Uuid) -> AppResult<FriendRequestRow> {
    let query = format!("{FRIEND_REQUEST_SELECT} WHERE r.id = $1");
    sqlx::query_as::<_, FriendRequestRow>(&query)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("friend request not found".into()))
}

/// `POST /friends/requests` — send a friend request. Auto-accepts when the
/// target already has a pending request to the caller.
pub async fn create_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateFriendRequest>,
) -> AppResult<Json<FriendRequestDto>> {
    let me = auth.id();
    let target = body.to_user_id;

    if me == target {
        return Err(AppError::BadRequest("you cannot add yourself".into()));
    }

    let target_exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(target)
            .fetch_one(&state.pool)
            .await?;
    if !target_exists {
        return Err(AppError::NotFound("user not found".into()));
    }

    let already_friends: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM friendships
            WHERE user_a_id = LEAST($1, $2) AND user_b_id = GREATEST($1, $2)
        )
        "#,
    )
    .bind(me)
    .bind(target)
    .fetch_one(&state.pool)
    .await?;
    if already_friends {
        return Err(AppError::Conflict("you are already friends".into()));
    }

    // If the target already asked us, accept that request instead.
    let reverse: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM friend_requests
        WHERE from_user_id = $1 AND to_user_id = $2 AND status = 'pending'
        "#,
    )
    .bind(target)
    .bind(me)
    .fetch_optional(&state.pool)
    .await?;

    if let Some(reverse_id) = reverse {
        return accept_internal(&state, me, reverse_id).await.map(Json);
    }

    let inserted = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO friend_requests (from_user_id, to_user_id, status)
        VALUES ($1, $2, 'pending')
        RETURNING id
        "#,
    )
    .bind(me)
    .bind(target)
    .fetch_one(&state.pool)
    .await;

    let request_id = match inserted {
        Ok(id) => id,
        Err(err) if is_unique_violation(&err) => {
            return Err(AppError::Conflict("a request is already pending".into()));
        }
        Err(err) => return Err(err.into()),
    };

    Metrics::incr(&state.metrics.friend_requests_created);
    let dto = FriendRequestDto::from(load_request(&state, request_id).await?);

    state.realtime.signaling.send_to(
        target,
        ServerEvent::FriendRequest {
            request_id,
            from_user: dto.from_user.clone(),
        },
    );

    Ok(Json(dto))
}

/// `GET /friends/requests/incoming`
pub async fn incoming(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<FriendRequestDto>>> {
    let query = format!(
        "{FRIEND_REQUEST_SELECT} WHERE r.to_user_id = $1 AND r.status = 'pending' ORDER BY r.created_at DESC"
    );
    let rows = sqlx::query_as::<_, FriendRequestRow>(&query)
        .bind(auth.id())
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

/// `GET /friends/requests/outgoing`
pub async fn outgoing(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<FriendRequestDto>>> {
    let query = format!(
        "{FRIEND_REQUEST_SELECT} WHERE r.from_user_id = $1 AND r.status = 'pending' ORDER BY r.created_at DESC"
    );
    let rows = sqlx::query_as::<_, FriendRequestRow>(&query)
        .bind(auth.id())
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn accept_internal(
    state: &AppState,
    me: Uuid,
    request_id: Uuid,
) -> AppResult<FriendRequestDto> {
    let mut tx = state.pool.begin().await?;

    let request = sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT from_user_id, to_user_id, status FROM friend_requests WHERE id = $1 FOR UPDATE",
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("friend request not found".into()))?;

    let (from_user_id, to_user_id, status) = request;
    if to_user_id != me {
        return Err(AppError::Forbidden);
    }
    if status != "pending" {
        return Err(AppError::Conflict("request is not pending".into()));
    }

    sqlx::query("UPDATE friend_requests SET status = 'accepted', updated_at = now() WHERE id = $1")
        .bind(request_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        r#"
        INSERT INTO friendships (user_a_id, user_b_id)
        VALUES (LEAST($1, $2), GREATEST($1, $2))
        ON CONFLICT (user_a_id, user_b_id) DO NOTHING
        "#,
    )
    .bind(from_user_id)
    .bind(to_user_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let dto = FriendRequestDto::from(load_request(state, request_id).await?);
    state.realtime.signaling.send_to(
        from_user_id,
        ServerEvent::FriendRequestAccepted {
            request_id,
            by_user: dto.to_user.clone(),
        },
    );

    // Nudge BOTH new friends to reload in real time. The requester gets the
    // FriendRequestAccepted above, but the accepter would otherwise only learn
    // of the new friend via the HTTP response it is already handling; pushing a
    // presence update to each side's friends (which now include each other)
    // guarantees the new friend appears immediately on both, without reopening
    // the menu.
    if let Err(e) = presence::broadcast_presence(state, from_user_id).await {
        tracing::warn!(error = %e, user_id = %from_user_id, "failed to broadcast presence after friend request accept");
    }
    if let Err(e) = presence::broadcast_presence(state, to_user_id).await {
        tracing::warn!(error = %e, user_id = %to_user_id, "failed to broadcast presence after friend request accept");
    }

    Ok(dto)
}

/// `POST /friends/requests/:id/accept`
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<FriendRequestDto>> {
    let dto = accept_internal(&state, auth.id(), id).await?;
    Ok(Json(dto))
}

/// `POST /friends/requests/:id/decline`
pub async fn decline(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<FriendRequestDto>> {
    let request = load_request(&state, id).await?;
    if request.to_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if request.status != "pending" {
        return Err(AppError::Conflict("request is not pending".into()));
    }

    sqlx::query("UPDATE friend_requests SET status = 'declined', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    state.realtime.signaling.send_to(
        request.from_id,
        ServerEvent::FriendRequestDeclined { request_id: id },
    );

    Ok(Json(FriendRequestDto::from(
        load_request(&state, id).await?,
    )))
}

/// `DELETE /friends/:userId`
pub async fn remove_friend(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(other): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let result = sqlx::query(
        r#"
        DELETE FROM friendships
        WHERE user_a_id = LEAST($1, $2) AND user_b_id = GREATEST($1, $2)
        "#,
    )
    .bind(auth.id())
    .bind(other)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("friendship not found".into()));
    }

    state
        .realtime
        .signaling
        .send_to(other, ServerEvent::FriendRemoved { user_id: auth.id() });

    Ok(Json(serde_json::json!({ "removed": true })))
}
