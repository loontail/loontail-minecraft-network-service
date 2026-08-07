use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::AssertSqlSafe;
use sqlx::PgPool;
use uuid::Uuid;

use loontail_core::auth::AuthUser;
use loontail_core::error::{is_unique_violation, AppError, AppResult};
use loontail_core::models::{InvitePolicy, JoinTicketDto, RequestStatus, UserDto};
use loontail_core::AppState;
use loontail_core::Metrics;
use loontail_core::ServerEvent;

use crate::join::{fetch_user_dto, issue_ticket_and_relay_tx};
use crate::queries::{are_friends, user_exists};
use crate::worlds;

/// One row carries the invite plus all three participants' [`UserDto`] columns, so a
/// list costs one query instead of 3N follow-up user reads (mirrors
/// `friends::FRIEND_REQUEST_SELECT`). `password_hash` is never selected. The joins are
/// INNER, which is total: every `*_user_id` is a `users(id)` FK with `ON DELETE
/// CASCADE`, so a row can never outlive its participants.
const INVITE_SELECT: &str = r#"
    SELECT
        i.id, i.status, i.world_session_id, i.host_user_id, i.inviter_user_id,
        i.invitee_user_id, i.created_at, i.expires_at,
        iu.minecraft_uuid AS inviter_minecraft_uuid, iu.username AS inviter_username,
        vu.minecraft_uuid AS invitee_minecraft_uuid, vu.username AS invitee_username,
        hu.minecraft_uuid AS host_minecraft_uuid, hu.username AS host_username
    FROM world_invites i
    JOIN users iu ON iu.id = i.inviter_user_id
    JOIN users vu ON vu.id = i.invitee_user_id
    JOIN users hu ON hu.id = i.host_user_id
"#;

#[derive(sqlx::FromRow)]
struct InviteRow {
    id: Uuid,
    status: RequestStatus,
    world_session_id: Uuid,
    host_user_id: Uuid,
    inviter_user_id: Uuid,
    invitee_user_id: Uuid,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    inviter_minecraft_uuid: Option<String>,
    inviter_username: String,
    invitee_minecraft_uuid: Option<String>,
    invitee_username: String,
    host_minecraft_uuid: Option<String>,
    host_username: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldInviteDto {
    pub id: Uuid,
    pub status: RequestStatus,
    pub world_session_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub requires_host_approval: bool,
    pub inviter: UserDto,
    pub invitee: UserDto,
    pub host: UserDto,
}

impl From<InviteRow> for WorldInviteDto {
    fn from(row: InviteRow) -> Self {
        WorldInviteDto {
            requires_host_approval: row.status == RequestStatus::PendingApproval,
            id: row.id,
            status: row.status,
            world_session_id: row.world_session_id,
            created_at: row.created_at,
            expires_at: row.expires_at,
            inviter: UserDto {
                id: row.inviter_user_id,
                minecraft_uuid: row.inviter_minecraft_uuid,
                username: row.inviter_username,
            },
            invitee: UserDto {
                id: row.invitee_user_id,
                minecraft_uuid: row.invitee_minecraft_uuid,
                username: row.invitee_username,
            },
            host: UserDto {
                id: row.host_user_id,
                minecraft_uuid: row.host_minecraft_uuid,
                username: row.host_username,
            },
        }
    }
}

async fn load_invite(pool: &PgPool, id: Uuid) -> AppResult<InviteRow> {
    let query = format!("{INVITE_SELECT} WHERE i.id = $1");
    sqlx::query_as::<_, InviteRow>(AssertSqlSafe(query))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("invite not found".into()))
}

/// [`load_invite`] with the row locked for the rest of the transaction, so a
/// concurrent accept serialises behind us and observes the committed status instead
/// of racing the same `pending` read (BE-02).
///
/// `OF i` is load-bearing: the select joins `users` three ways and a bare `FOR UPDATE`
/// would lock those rows too, serialising unrelated traffic on the participants.
async fn load_invite_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> AppResult<InviteRow> {
    let query = format!("{INVITE_SELECT} WHERE i.id = $1 FOR UPDATE OF i");
    sqlx::query_as::<_, InviteRow>(AssertSqlSafe(query))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::NotFound("invite not found".into()))
}

async fn list_by(
    state: &AppState,
    query: String,
    user_id: Uuid,
) -> AppResult<Json<Vec<WorldInviteDto>>> {
    let rows = sqlx::query_as::<_, InviteRow>(AssertSqlSafe(query))
        .bind(user_id)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows.into_iter().map(WorldInviteDto::from).collect()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvite {
    pub invitee_user_id: Uuid,
}

/// `POST /world-sessions/:id/invites` — the host invites their own friends
/// directly. Under `friends_of_friends`, a friend the host trusts may invite
/// their own friends; an invitee not already the host's friend is held as
/// `pending_approval` until the host approves.
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(world_id): Path<Uuid>,
    Json(body): Json<CreateInvite>,
) -> AppResult<Json<WorldInviteDto>> {
    let caller = auth.id();
    let invitee = body.invitee_user_id;

    let world = worlds::load_open_world_session(&state.pool, world_id).await?;
    let host = world.host_user_id;

    if invitee == caller {
        return Err(AppError::BadRequest("you cannot invite yourself".into()));
    }
    if invitee == host {
        return Err(AppError::BadRequest(
            "the host is already in the world".into(),
        ));
    }
    if !user_exists(&state.pool, invitee).await? {
        return Err(AppError::NotFound("user not found".into()));
    }

    let status = if caller == host {
        if !are_friends(&state.pool, host, invitee).await? {
            return Err(AppError::Forbidden);
        }
        RequestStatus::Pending
    } else {
        // Friend-of-friend invite: allowed only when the policy opts in and the
        // inviter is the host's friend inviting one of their own friends.
        if world.invite_policy != InvitePolicy::FriendsOfFriends {
            return Err(AppError::Forbidden);
        }
        if !are_friends(&state.pool, host, caller).await? {
            return Err(AppError::Forbidden);
        }
        if !are_friends(&state.pool, caller, invitee).await? {
            return Err(AppError::Forbidden);
        }
        // The host's own friends skip approval; true friends-of-friends wait.
        if are_friends(&state.pool, host, invitee).await? {
            RequestStatus::Pending
        } else {
            RequestStatus::PendingApproval
        }
    };

    let ttl =
        Duration::from_std(state.config.invite_ttl).unwrap_or_else(|_| Duration::seconds(600));
    let expires_at = Utc::now() + ttl;

    // Free the unique active slot from an earlier lapsed invite for this
    // (world, invitee); the partial unique index would otherwise block a new
    // one forever while the expired one stays hidden from every list.
    sqlx::query(
        r#"
        UPDATE world_invites SET status = 'expired', updated_at = now()
        WHERE world_session_id = $1 AND invitee_user_id = $2
          AND status IN ('pending', 'pending_approval') AND expires_at <= now()
        "#,
    )
    .bind(world_id)
    .bind(invitee)
    .execute(&state.pool)
    .await?;

    let inserted = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO world_invites
            (world_session_id, host_user_id, inviter_user_id, invitee_user_id, status, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(world_id)
    .bind(host)
    .bind(caller)
    .bind(invitee)
    .bind(status)
    .bind(expires_at)
    .fetch_one(&state.pool)
    .await;

    let invite_id = match inserted {
        Ok(id) => id,
        Err(err) if is_unique_violation(&err) => {
            return Err(AppError::Conflict(
                "an invite for this player is already pending".into(),
            ));
        }
        Err(err) => return Err(err.into()),
    };

    let dto = WorldInviteDto::from(load_invite(&state.pool, invite_id).await?);

    if status == RequestStatus::Pending {
        state.realtime.signaling.send_to(
            invitee,
            ServerEvent::WorldInvite {
                invite_id,
                world_session_id: world_id,
                inviter: dto.inviter.clone(),
                host: dto.host.clone(),
            },
        );
    } else {
        state.realtime.signaling.send_to(
            host,
            ServerEvent::InviteApprovalRequest {
                invite_id,
                world_session_id: world_id,
                inviter: dto.inviter.clone(),
                invitee: dto.invitee.clone(),
            },
        );
    }

    Ok(Json(dto))
}

/// `GET /invites/incoming` — invites addressed to me, awaiting accept/decline.
pub async fn incoming(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<WorldInviteDto>>> {
    let query = format!(
        "{INVITE_SELECT} WHERE i.invitee_user_id = $1 AND i.status = 'pending' \
         AND i.expires_at > now() ORDER BY i.created_at DESC"
    );
    list_by(&state, query, auth.id()).await
}

/// `GET /invites/outgoing` — invites I sent that are still active.
pub async fn outgoing(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<WorldInviteDto>>> {
    let query = format!(
        "{INVITE_SELECT} WHERE i.inviter_user_id = $1 \
         AND i.status IN ('pending', 'pending_approval') AND i.expires_at > now() \
         ORDER BY i.created_at DESC"
    );
    list_by(&state, query, auth.id()).await
}

/// `GET /invites/pending-approval` — friend-of-friend invites awaiting my
/// approval as host.
pub async fn pending_approval(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<WorldInviteDto>>> {
    let query = format!(
        "{INVITE_SELECT} WHERE i.host_user_id = $1 AND i.status = 'pending_approval' \
         AND i.expires_at > now() ORDER BY i.created_at DESC"
    );
    list_by(&state, query, auth.id()).await
}

/// `POST /invites/:id/accept` — invitee accepts; returns a join ticket and cues
/// the host to open its relay tunnel.
///
/// The whole admission runs in one transaction whose gate is the row lock plus the
/// conditional status transition, so N concurrent accepts mint exactly one ticket +
/// relay session and the losers get a 409 (BE-02).
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<JoinTicketDto>> {
    let mut tx = state.pool.begin().await?;
    let row = load_invite_for_update(&mut tx, id).await?;
    if row.invitee_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != RequestStatus::Pending {
        return Err(AppError::Conflict("invite is not ready to accept".into()));
    }
    if row.expires_at <= Utc::now() {
        return Err(AppError::Conflict("invite has expired".into()));
    }

    // why: every read below runs on `tx`, never on `state.pool`. Borrowing a second
    // pooled connection while this transaction holds one makes the request need two, so
    // `max_connections` concurrent accepts would deadlock the pool for `acquire_timeout`.
    let world = worlds::load_open_world_session(&mut *tx, row.world_session_id).await?;

    // Re-validate the friend-of-friend chain at admission: a host_only flip or a
    // severed friendship since the invite was issued must block the guest now.
    // Host-issued invites (inviter == host) are not FoF and skip this.
    if row.inviter_user_id != row.host_user_id {
        if world.invite_policy != InvitePolicy::FriendsOfFriends {
            return Err(AppError::Forbidden);
        }
        if !are_friends(&mut *tx, row.host_user_id, row.inviter_user_id).await? {
            return Err(AppError::Forbidden);
        }
        if !are_friends(&mut *tx, row.inviter_user_id, row.invitee_user_id).await? {
            return Err(AppError::Forbidden);
        }
    }

    if world.current_players >= world.max_players {
        return Err(AppError::Conflict("world is full".into()));
    }

    let transitioned = sqlx::query(
        "UPDATE world_invites SET status = 'accepted', updated_at = now() \
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if transitioned.rows_affected() != 1 {
        return Err(AppError::Conflict("invite is not ready to accept".into()));
    }

    let ticket = issue_ticket_and_relay_tx(
        &state,
        &mut tx,
        row.invitee_user_id,
        row.host_user_id,
        row.world_session_id,
    )
    .await?;

    tx.commit().await?;
    Metrics::incr(&state.metrics.join_tickets_issued);

    let guest_dto = fetch_user_dto(&state.pool, row.invitee_user_id).await?;
    state.realtime.signaling.send_to(
        row.host_user_id,
        ServerEvent::GuestArriving {
            relay_session_id: ticket.relay_session_id,
            world_session_id: row.world_session_id,
            guest_user: guest_dto,
        },
    );

    Ok(Json(ticket))
}

/// `POST /invites/:id/decline` — invitee declines.
pub async fn decline(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<WorldInviteDto>> {
    let row = load_invite(&state.pool, id).await?;
    if row.invitee_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != RequestStatus::Pending {
        return Err(AppError::Conflict("invite is not pending".into()));
    }
    let transitioned = sqlx::query(
        "UPDATE world_invites SET status = 'declined', updated_at = now() \
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    if transitioned.rows_affected() == 0 {
        return Err(AppError::Conflict("invite is not pending".into()));
    }
    Ok(Json(WorldInviteDto::from(
        load_invite(&state.pool, id).await?,
    )))
}

/// `POST /invites/:id/approve` — host approves a friend-of-friend invite,
/// releasing it to the invitee.
pub async fn approve(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<WorldInviteDto>> {
    let row = load_invite(&state.pool, id).await?;
    if row.host_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != RequestStatus::PendingApproval {
        return Err(AppError::Conflict("invite is not awaiting approval".into()));
    }
    if row.expires_at <= Utc::now() {
        return Err(AppError::Conflict("invite has expired".into()));
    }

    // why: the status predicate is the concurrency gate — two simultaneous approvals
    // must release the invite (and push WorldInvite to the invitee) exactly once.
    let transitioned = sqlx::query(
        "UPDATE world_invites SET status = 'pending', updated_at = now() \
         WHERE id = $1 AND status = 'pending_approval'",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    if transitioned.rows_affected() == 0 {
        return Err(AppError::Conflict("invite is not awaiting approval".into()));
    }

    let dto = WorldInviteDto::from(load_invite(&state.pool, id).await?);
    state.realtime.signaling.send_to(
        row.invitee_user_id,
        ServerEvent::WorldInvite {
            invite_id: id,
            world_session_id: row.world_session_id,
            inviter: dto.inviter.clone(),
            host: dto.host.clone(),
        },
    );
    Ok(Json(dto))
}

/// `DELETE /invites/:id` — the host or the inviter cancels a pending invite.
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let row = load_invite(&state.pool, id).await?;
    if row.host_user_id != auth.id() && row.inviter_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    let result = sqlx::query(
        "UPDATE world_invites SET status = 'revoked', updated_at = now() \
         WHERE id = $1 AND status IN ('pending', 'pending_approval')",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Conflict("invite is no longer active".into()));
    }
    Ok(Json(serde_json::json!({ "revoked": true })))
}
