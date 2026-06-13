use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use loontail_core::auth::{generate_token, hash_token, AuthUser};
use loontail_core::error::{AppError, AppResult};
use loontail_core::models::{JoinTicketDto, User, UserDto, UserStatus};
use loontail_core::AppState;
use loontail_core::Metrics;
use loontail_core::ServerEvent;

use crate::presence::effective_status;
use crate::worlds;

// --- Shared helpers --------------------------------------------------------

pub(crate) async fn fetch_user_dto(pool: &PgPool, id: Uuid) -> AppResult<UserDto> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".into()))?;
    Ok(UserDto::from(user))
}

pub(crate) async fn are_friends(pool: &PgPool, a: Uuid, b: Uuid) -> AppResult<bool> {
    let friends: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM friendships
            WHERE user_a_id = LEAST($1, $2) AND user_b_id = GREATEST($1, $2)
        )
        "#,
    )
    .bind(a)
    .bind(b)
    .fetch_one(pool)
    .await?;
    Ok(friends)
}

async fn host_effective_status(state: &AppState, host_id: Uuid) -> AppResult<UserStatus> {
    let row = sqlx::query_as::<_, (String, DateTime<Utc>)>(
        "SELECT status, last_heartbeat_at FROM presence WHERE user_id = $1",
    )
    .bind(host_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(match row {
        Some((status, heartbeat)) => effective_status(
            UserStatus::from_db(&status),
            heartbeat,
            state.config.heartbeat_timeout,
        ),
        None => UserStatus::Offline,
    })
}

/// Whether `requester` is a friend of someone currently AND LIVELY connected to
/// `world_id` as a guest — the trust link a friend-of-friend join/invite rides
/// on. The liveness predicate (heartbeat within the timeout) mirrors the
/// presence query, so a stale `active` relay row left by an unclean disconnect
/// cannot keep the friend-of-friend door open after the guest is gone.
pub(crate) async fn is_friend_of_active_member(
    state: &AppState,
    requester: Uuid,
    world_id: Uuid,
) -> AppResult<bool> {
    let timeout_secs = state.config.heartbeat_timeout.as_secs_f64();
    let exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM relay_sessions rs
            JOIN friendships f
              ON f.user_a_id = LEAST(rs.guest_user_id, $1)
             AND f.user_b_id = GREATEST(rs.guest_user_id, $1)
            JOIN presence p ON p.user_id = rs.guest_user_id
            WHERE rs.world_session_id = $2
              AND rs.status = 'active'
              AND rs.guest_user_id IS NOT NULL
              AND p.status <> 'offline'
              AND p.last_heartbeat_at > now() - ($3 * interval '1 second')
        )
        "#,
    )
    .bind(requester)
    .bind(world_id)
    .bind(timeout_secs)
    .fetch_one(&state.pool)
    .await?;
    Ok(exists)
}

/// Create a join ticket and its pending relay session for `guest` joining
/// `host`'s world. Returns the ticket (with raw token) for delivery.
pub(crate) async fn issue_ticket_and_relay(
    state: &AppState,
    guest_id: Uuid,
    host_id: Uuid,
    world_session_id: Uuid,
) -> AppResult<JoinTicketDto> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let ttl =
        Duration::from_std(state.config.join_ticket_ttl).unwrap_or_else(|_| Duration::seconds(60));
    let expires_at = Utc::now() + ttl;

    let mut tx = state.pool.begin().await?;

    // The world's invite policy plus the host's reported version/loader (for the guest's
    // authoritative compatibility gate), read together in one round-trip inside the tx.
    let (invite_policy, host_minecraft_version, host_loader): (
        String,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        r#"
            SELECT ws.invite_policy, p.minecraft_version, p.loader
            FROM world_sessions ws
            LEFT JOIN presence p ON p.user_id = ws.host_user_id
            WHERE ws.id = $1
            "#,
    )
    .bind(world_session_id)
    .fetch_one(&mut *tx)
    .await?;

    let ticket_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO join_tickets (user_id, world_session_id, token_hash, expires_at)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(guest_id)
    .bind(world_session_id)
    .bind(&token_hash)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await?;

    let relay_session_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO relay_sessions
            (world_session_id, host_user_id, guest_user_id, join_ticket_id, status)
        VALUES ($1, $2, $3, $4, 'pending')
        RETURNING id
        "#,
    )
    .bind(world_session_id)
    .bind(host_id)
    .bind(guest_id)
    .bind(ticket_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Metrics::incr(&state.metrics.join_tickets_issued);

    Ok(JoinTicketDto {
        ticket: token,
        relay_session_id,
        world_session_id,
        host_user_id: host_id,
        expires_at,
        invite_policy,
        host_minecraft_version,
        host_loader,
    })
}

// --- Direct join (joinable) ------------------------------------------------

/// `POST /world-sessions/:id/join-ticket` — friend is `joinable`, so a guest
/// gets a ticket immediately without host approval.
pub async fn create_join_ticket(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(world_id): Path<Uuid>,
) -> AppResult<Json<JoinTicketDto>> {
    let guest = auth.id();
    let world = worlds::open_world_session(&state.pool, world_id).await?;
    let host = world.host_user_id;

    if host == guest {
        return Err(AppError::BadRequest(
            "you cannot join your own world".into(),
        ));
    }
    if !are_friends(&state.pool, guest, host).await? {
        return Err(AppError::Forbidden);
    }
    if host_effective_status(&state, host).await? != UserStatus::Joinable {
        return Err(AppError::Conflict(
            "host is not open for free join; send a join request instead".into(),
        ));
    }
    if world.current_players >= world.max_players {
        return Err(AppError::Conflict("world is full".into()));
    }

    let ticket = issue_ticket_and_relay(&state, guest, host, world_id).await?;
    let guest_dto = fetch_user_dto(&state.pool, guest).await?;

    state.realtime.signaling.send_to(
        host,
        ServerEvent::GuestArriving {
            relay_session_id: ticket.relay_session_id,
            world_session_id: world_id,
            guest_user: guest_dto,
        },
    );

    Ok(Json(ticket))
}

// --- Request join (inWorld) ------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequestDto {
    pub id: Uuid,
    pub status: String,
    pub world_session_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub requester: UserDto,
    pub host: UserDto,
}

#[derive(sqlx::FromRow)]
struct JoinRequestRow {
    id: Uuid,
    status: String,
    world_session_id: Uuid,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    requester_user_id: Uuid,
    host_user_id: Uuid,
}

async fn build_join_request_dto(pool: &PgPool, row: JoinRequestRow) -> AppResult<JoinRequestDto> {
    let requester = fetch_user_dto(pool, row.requester_user_id).await?;
    let host = fetch_user_dto(pool, row.host_user_id).await?;
    Ok(JoinRequestDto {
        id: row.id,
        status: row.status,
        world_session_id: row.world_session_id,
        created_at: row.created_at,
        expires_at: row.expires_at,
        requester,
        host,
    })
}

async fn load_join_request(pool: &PgPool, id: Uuid) -> AppResult<JoinRequestRow> {
    sqlx::query_as::<_, JoinRequestRow>(
        r#"
        SELECT id, status, world_session_id, created_at, expires_at,
               requester_user_id, host_user_id
        FROM join_requests WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("join request not found".into()))
}

/// `POST /world-sessions/:id/join-requests` — friend is `inWorld`, so the
/// guest asks the host for approval.
pub async fn create_join_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(world_id): Path<Uuid>,
) -> AppResult<Json<JoinRequestDto>> {
    let requester = auth.id();
    let world = worlds::open_world_session(&state.pool, world_id).await?;
    let host = world.host_user_id;

    if host == requester {
        return Err(AppError::BadRequest(
            "you cannot join your own world".into(),
        ));
    }
    // The host's own friends may always ask. A friend-of-friend may ask only when
    // the world opts in AND they are a friend of someone already in the world.
    if !are_friends(&state.pool, requester, host).await? {
        if world.invite_policy != "friends_of_friends" {
            return Err(AppError::Forbidden);
        }
        if !is_friend_of_active_member(&state, requester, world_id).await? {
            return Err(AppError::Forbidden);
        }
    }
    if !host_effective_status(&state, host).await?.is_in_world() {
        return Err(AppError::Conflict("host is not in a world".into()));
    }

    let ttl =
        Duration::from_std(state.config.join_request_ttl).unwrap_or_else(|_| Duration::seconds(60));
    let expires_at = Utc::now() + ttl;

    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO join_requests
            (requester_user_id, host_user_id, world_session_id, status, expires_at)
        VALUES ($1, $2, $3, 'pending', $4)
        RETURNING id
        "#,
    )
    .bind(requester)
    .bind(host)
    .bind(world_id)
    .bind(expires_at)
    .fetch_one(&state.pool)
    .await?;

    let dto =
        build_join_request_dto(&state.pool, load_join_request(&state.pool, id).await?).await?;

    state.realtime.signaling.send_to(
        host,
        ServerEvent::JoinRequest {
            request_id: id,
            world_session_id: world_id,
            from_user: dto.requester.clone(),
        },
    );

    Ok(Json(dto))
}

/// `GET /join-requests/incoming` — host's pending, unexpired join requests.
pub async fn incoming(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<JoinRequestDto>>> {
    let rows = sqlx::query_as::<_, JoinRequestRow>(
        r#"
        SELECT id, status, world_session_id, created_at, expires_at,
               requester_user_id, host_user_id
        FROM join_requests
        WHERE host_user_id = $1 AND status = 'pending' AND expires_at > now()
        ORDER BY created_at DESC
        "#,
    )
    .bind(auth.id())
    .fetch_all(&state.pool)
    .await?;

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        result.push(build_join_request_dto(&state.pool, row).await?);
    }
    Ok(Json(result))
}

/// `POST /join-requests/:id/accept` — host approves; guest receives a ticket
/// over signaling.
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<JoinRequestDto>> {
    let row = load_join_request(&state.pool, id).await?;
    if row.host_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != "pending" {
        return Err(AppError::Conflict("join request is not pending".into()));
    }
    if row.expires_at <= Utc::now() {
        return Err(AppError::Conflict("join request has expired".into()));
    }

    // World must still be open and have room.
    let world = worlds::open_world_session(&state.pool, row.world_session_id).await?;

    // Re-validate friend-of-friend eligibility at the moment of admission: a
    // request created while the policy/trust allowed it must not still admit the
    // guest after the host flips back to host_only or the trust link is severed
    // (the host approving does not re-establish a link that no longer exists).
    if !are_friends(&state.pool, row.requester_user_id, row.host_user_id).await?
        && (world.invite_policy != "friends_of_friends"
            || !is_friend_of_active_member(&state, row.requester_user_id, row.world_session_id)
                .await?)
    {
        return Err(AppError::Forbidden);
    }

    if world.current_players >= world.max_players {
        return Err(AppError::Conflict("world is full".into()));
    }

    sqlx::query("UPDATE join_requests SET status = 'accepted', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    let ticket = issue_ticket_and_relay(
        &state,
        row.requester_user_id,
        row.host_user_id,
        row.world_session_id,
    )
    .await?;

    let guest_dto = fetch_user_dto(&state.pool, row.requester_user_id).await?;

    // Guest gets the ticket; host gets the cue to open its relay tunnel.
    state.realtime.signaling.send_to(
        row.requester_user_id,
        ServerEvent::JoinRequestAccepted {
            request_id: id,
            ticket: ticket.clone(),
        },
    );
    state.realtime.signaling.send_to(
        row.host_user_id,
        ServerEvent::GuestArriving {
            relay_session_id: ticket.relay_session_id,
            world_session_id: row.world_session_id,
            guest_user: guest_dto,
        },
    );

    let dto =
        build_join_request_dto(&state.pool, load_join_request(&state.pool, id).await?).await?;
    Ok(Json(dto))
}

/// `POST /join-requests/:id/decline` — host rejects the request.
pub async fn decline(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<JoinRequestDto>> {
    let row = load_join_request(&state.pool, id).await?;
    if row.host_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != "pending" {
        return Err(AppError::Conflict("join request is not pending".into()));
    }

    sqlx::query("UPDATE join_requests SET status = 'declined', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    let requester = row.requester_user_id;
    let dto =
        build_join_request_dto(&state.pool, load_join_request(&state.pool, id).await?).await?;

    state.realtime.signaling.send_to(
        requester,
        ServerEvent::JoinRequestDeclined { request_id: id },
    );

    Ok(Json(dto))
}
