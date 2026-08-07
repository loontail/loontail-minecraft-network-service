use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::AssertSqlSafe;
use sqlx::PgPool;
use uuid::Uuid;

use loontail_core::auth::{generate_token, hash_token, AuthUser};
use loontail_core::error::{is_unique_violation, AppError, AppResult};
use loontail_core::models::{
    InvitePolicy, JoinTicketDto, PresenceStatus, RequestStatus, User, UserDto,
};
use loontail_core::AppState;
use loontail_core::Metrics;
use loontail_core::ServerEvent;

use crate::presence;
use crate::queries::are_friends;
use crate::worlds;

pub(crate) async fn fetch_user_dto(pool: &PgPool, id: Uuid) -> AppResult<UserDto> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".into()))?;
    Ok(UserDto::from(user))
}

async fn host_effective_status(state: &AppState, host_id: Uuid) -> AppResult<PresenceStatus> {
    presence::effective_status_for(state, host_id).await
}

/// Whether `requester` is a friend of someone LIVELY connected to `world_id` as
/// a guest — the trust link a friend-of-friend join/invite rides on. Liveness
/// (heartbeat within `heartbeat_timeout`) is required so a stale `active` relay row
/// from an unclean disconnect cannot keep the FoF door open after the guest is gone.
///
/// Runs on any executor. why: an admission path that already holds a transaction MUST
/// pass `&mut *tx` here — reaching back for `&state.pool` while holding one pooled
/// connection makes the request need two, which self-deadlocks the pool under load.
pub(crate) async fn is_friend_of_active_member<'e, E>(
    executor: E,
    heartbeat_timeout: std::time::Duration,
    requester: Uuid,
    world_id: Uuid,
) -> AppResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let timeout_secs = heartbeat_timeout.as_secs_f64();
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
    .fetch_one(executor)
    .await?;
    Ok(exists)
}

/// Create a join ticket + its pending relay session; returns the ticket with
/// the raw token (delivered once, then hashed at rest).
pub(crate) async fn issue_ticket_and_relay(
    state: &AppState,
    guest_id: Uuid,
    host_id: Uuid,
    world_session_id: Uuid,
) -> AppResult<JoinTicketDto> {
    let mut tx = state.pool.begin().await?;
    let ticket =
        issue_ticket_and_relay_tx(state, &mut tx, guest_id, host_id, world_session_id).await?;
    tx.commit().await?;
    Metrics::incr(&state.metrics.join_tickets_issued);
    Ok(ticket)
}

/// [`issue_ticket_and_relay`] on the caller's transaction, so minting the ticket +
/// relay row commits atomically with whatever status transition authorised it. The
/// caller owns the commit AND the metric — a ticket must never be counted (or
/// announced over signaling) before its transaction lands.
pub(crate) async fn issue_ticket_and_relay_tx(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    guest_id: Uuid,
    host_id: Uuid,
    world_session_id: Uuid,
) -> AppResult<JoinTicketDto> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let ttl =
        Duration::from_std(state.config.join_ticket_ttl).unwrap_or_else(|_| Duration::seconds(60));
    let expires_at = Utc::now() + ttl;

    // Invite policy + the host's reported version/loader (the guest's
    // authoritative compatibility gate), read together in one round-trip.
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
    .fetch_one(&mut **tx)
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
    .fetch_one(&mut **tx)
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
    .fetch_one(&mut **tx)
    .await?;

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

/// `POST /world-sessions/:id/join-ticket` — friend is `joinable`, so a guest
/// gets a ticket immediately without host approval.
pub async fn create_join_ticket(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(world_id): Path<Uuid>,
) -> AppResult<Json<JoinTicketDto>> {
    let guest = auth.id();
    let world = worlds::load_open_world_session(&state.pool, world_id).await?;
    let host = world.host_user_id;

    if host == guest {
        return Err(AppError::BadRequest(
            "you cannot join your own world".into(),
        ));
    }
    if !are_friends(&state.pool, guest, host).await? {
        return Err(AppError::Forbidden);
    }
    if host_effective_status(&state, host).await? != PresenceStatus::Joinable {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequestDto {
    pub id: Uuid,
    pub status: RequestStatus,
    pub world_session_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub requester: UserDto,
    pub host: UserDto,
}

/// One row carries the request plus both participants' [`UserDto`] columns, so a list
/// costs one query instead of 2N follow-up user reads (mirrors
/// `friends::FRIEND_REQUEST_SELECT`). `password_hash` is never selected. The joins are
/// INNER, which is total: both `*_user_id` columns are `users(id)` FKs with `ON DELETE
/// CASCADE`, so a row can never outlive its participants.
const JOIN_REQUEST_SELECT: &str = r#"
    SELECT
        j.id, j.status, j.world_session_id, j.created_at, j.expires_at,
        j.requester_user_id, j.host_user_id,
        ru.minecraft_uuid AS requester_minecraft_uuid, ru.username AS requester_username,
        hu.minecraft_uuid AS host_minecraft_uuid, hu.username AS host_username
    FROM join_requests j
    JOIN users ru ON ru.id = j.requester_user_id
    JOIN users hu ON hu.id = j.host_user_id
"#;

#[derive(sqlx::FromRow)]
struct JoinRequestRow {
    id: Uuid,
    status: RequestStatus,
    world_session_id: Uuid,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    requester_user_id: Uuid,
    host_user_id: Uuid,
    requester_minecraft_uuid: Option<String>,
    requester_username: String,
    host_minecraft_uuid: Option<String>,
    host_username: String,
}

impl From<JoinRequestRow> for JoinRequestDto {
    fn from(row: JoinRequestRow) -> Self {
        JoinRequestDto {
            id: row.id,
            status: row.status,
            world_session_id: row.world_session_id,
            created_at: row.created_at,
            expires_at: row.expires_at,
            requester: UserDto {
                id: row.requester_user_id,
                minecraft_uuid: row.requester_minecraft_uuid,
                username: row.requester_username,
            },
            host: UserDto {
                id: row.host_user_id,
                minecraft_uuid: row.host_minecraft_uuid,
                username: row.host_username,
            },
        }
    }
}

async fn load_join_request(pool: &PgPool, id: Uuid) -> AppResult<JoinRequestRow> {
    let query = format!("{JOIN_REQUEST_SELECT} WHERE j.id = $1");
    sqlx::query_as::<_, JoinRequestRow>(AssertSqlSafe(query))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("join request not found".into()))
}

/// [`load_join_request`] with the row locked for the rest of the transaction, so a
/// concurrent accept serialises behind us and observes the committed status instead
/// of racing the same `pending` read (BE-02).
///
/// `OF j` is load-bearing: the select joins `users` twice and a bare `FOR UPDATE` would
/// lock those rows too, serialising unrelated traffic on the participants.
async fn load_join_request_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> AppResult<JoinRequestRow> {
    let query = format!("{JOIN_REQUEST_SELECT} WHERE j.id = $1 FOR UPDATE OF j");
    sqlx::query_as::<_, JoinRequestRow>(AssertSqlSafe(query))
        .bind(id)
        .fetch_optional(&mut **tx)
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
    let world = worlds::load_open_world_session(&state.pool, world_id).await?;
    let host = world.host_user_id;

    if host == requester {
        return Err(AppError::BadRequest(
            "you cannot join your own world".into(),
        ));
    }
    // The host's own friends may always ask. A friend-of-friend may ask only when
    // the world opts in AND they are a friend of someone already in the world.
    if !are_friends(&state.pool, requester, host).await? {
        if world.invite_policy != InvitePolicy::FriendsOfFriends {
            return Err(AppError::Forbidden);
        }
        if !is_friend_of_active_member(
            &state.pool,
            state.config.heartbeat_timeout,
            requester,
            world_id,
        )
        .await?
        {
            return Err(AppError::Forbidden);
        }
    }
    if !host_effective_status(&state, host).await?.is_in_world() {
        return Err(AppError::Conflict("host is not in a world".into()));
    }

    let ttl =
        Duration::from_std(state.config.join_request_ttl).unwrap_or_else(|_| Duration::seconds(60));
    let expires_at = Utc::now() + ttl;

    // Free the unique pending slot from an earlier lapsed request for this
    // (world, requester); the partial unique index would otherwise block a new one
    // forever while the expired row stays hidden from every list.
    sqlx::query(
        r#"
        UPDATE join_requests SET status = 'expired', updated_at = now()
        WHERE world_session_id = $1 AND requester_user_id = $2
          AND status = 'pending' AND expires_at <= now()
        "#,
    )
    .bind(world_id)
    .bind(requester)
    .execute(&state.pool)
    .await?;

    let inserted = sqlx::query_scalar::<_, Uuid>(
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
    .await;

    // why: the partial unique index is the gate. Without it a requester could loop this
    // endpoint, inserting a row and firing a signaling push to the host each time.
    let id = match inserted {
        Ok(id) => id,
        Err(err) if is_unique_violation(&err) => {
            return Err(AppError::Conflict(
                "a join request is already pending".into(),
            ));
        }
        Err(err) => return Err(err.into()),
    };

    let dto = JoinRequestDto::from(load_join_request(&state.pool, id).await?);

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
    let query = format!(
        "{JOIN_REQUEST_SELECT} WHERE j.host_user_id = $1 AND j.status = 'pending' \
         AND j.expires_at > now() ORDER BY j.created_at DESC"
    );
    let rows = sqlx::query_as::<_, JoinRequestRow>(AssertSqlSafe(query))
        .bind(auth.id())
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(rows.into_iter().map(JoinRequestDto::from).collect()))
}

/// `POST /join-requests/:id/accept` — host approves; guest receives a ticket
/// over signaling.
///
/// The whole admission runs in one transaction whose gate is the row lock plus the
/// conditional status transition, so N concurrent accepts mint exactly one ticket +
/// relay session and the losers get a 409 (BE-02).
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<JoinRequestDto>> {
    let mut tx = state.pool.begin().await?;
    let row = load_join_request_for_update(&mut tx, id).await?;
    if row.host_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != RequestStatus::Pending {
        return Err(AppError::Conflict("join request is not pending".into()));
    }
    if row.expires_at <= Utc::now() {
        return Err(AppError::Conflict("join request has expired".into()));
    }

    // why: every read below runs on `tx`, never on `state.pool`. Borrowing a second
    // pooled connection while this transaction holds one makes the request need two, so
    // `max_connections` concurrent accepts would deadlock the pool for `acquire_timeout`.
    let world = worlds::load_open_world_session(&mut *tx, row.world_session_id).await?;

    // Re-validate friend-of-friend eligibility at admission: a host_only flip or
    // a severed trust link since the request was made must block the guest now
    // (approving does not re-establish a link that no longer exists).
    let direct_friends = are_friends(&mut *tx, row.requester_user_id, row.host_user_id).await?;
    if !direct_friends {
        if world.invite_policy != InvitePolicy::FriendsOfFriends {
            return Err(AppError::Forbidden);
        }
        let via_member = is_friend_of_active_member(
            &mut *tx,
            state.config.heartbeat_timeout,
            row.requester_user_id,
            row.world_session_id,
        )
        .await?;
        if !via_member {
            return Err(AppError::Forbidden);
        }
    }

    if world.current_players >= world.max_players {
        return Err(AppError::Conflict("world is full".into()));
    }

    let transitioned = sqlx::query(
        "UPDATE join_requests SET status = 'accepted', updated_at = now() \
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if transitioned.rows_affected() != 1 {
        return Err(AppError::Conflict("join request is not pending".into()));
    }

    let ticket = issue_ticket_and_relay_tx(
        &state,
        &mut tx,
        row.requester_user_id,
        row.host_user_id,
        row.world_session_id,
    )
    .await?;

    tx.commit().await?;
    Metrics::incr(&state.metrics.join_tickets_issued);

    let guest_dto = fetch_user_dto(&state.pool, row.requester_user_id).await?;

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

    Ok(Json(JoinRequestDto::from(
        load_join_request(&state.pool, id).await?,
    )))
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
    if row.status != RequestStatus::Pending {
        return Err(AppError::Conflict("join request is not pending".into()));
    }

    let transitioned = sqlx::query(
        "UPDATE join_requests SET status = 'declined', updated_at = now() \
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    if transitioned.rows_affected() == 0 {
        return Err(AppError::Conflict("join request is not pending".into()));
    }

    let requester = row.requester_user_id;
    let dto = JoinRequestDto::from(load_join_request(&state.pool, id).await?);

    state.realtime.signaling.send_to(
        requester,
        ServerEvent::JoinRequestDeclined { request_id: id },
    );

    Ok(Json(dto))
}
